import { existsSync, lstatSync, mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { DatabaseSync } from 'node:sqlite';
import { join } from 'node:path';
import { knownSecrets } from './env.js';
import type { RunReport } from './types.js';

const KEY_PATTERN = /\ba4_(?:ak|sk|pk)_[A-Za-z0-9]{16,}\b/g;
const BEARER_PATTERN = /\b(Bearer\s+)[A-Za-z0-9._~+/=-]{20,}/g;

/**
 * Redaction for binary files such as SQLite session stores: each secret is
 * overwritten with the same number of bytes, so offsets and page layouts stay
 * valid. Works on latin1 strings, where one character is one byte.
 */
export function maskSameLength(bytes: string, secrets: string[] = knownSecrets()): string {
  const mask = (match: string) => 'X'.repeat(match.length);
  let out = bytes;
  for (const secret of secrets) out = out.split(secret).join(mask(secret));
  return out
    .replace(KEY_PATTERN, mask)
    .replace(BEARER_PATTERN, (match, prefix: string) => prefix + mask(match.slice(prefix.length)));
}

export function redact(text: string, secrets: string[] = knownSecrets()): string {
  let out = text;
  for (const secret of secrets) out = out.split(secret).join('[REDACTED]');
  return out.replace(KEY_PATTERN, '[REDACTED_A4_KEY]').replace(BEARER_PATTERN, '$1[REDACTED]');
}

/**
 * Console output, redacted like written artifacts. The weekly workflow runs
 * in a public repository, so its logs are public.
 */
export function print(text: string, stream: NodeJS.WriteStream = process.stdout): void {
  stream.write(redact(text));
}

export class RunDir {
  constructor(readonly path: string) {
    mkdirSync(path, { recursive: true });
  }

  writeText(name: string, content: string): void {
    const file = join(this.path, name);
    mkdirSync(join(file, '..'), { recursive: true });
    writeFileSync(file, redact(content));
  }

  writeJson(name: string, value: unknown): void {
    this.writeText(name, `${JSON.stringify(value, null, 2)}\n`);
  }

  writeJsonl(name: string, rows: unknown[]): void {
    this.writeText(name, rows.map((r) => JSON.stringify(r)).join('\n') + (rows.length ? '\n' : ''));
  }

  /**
   * Redact every file under a downloaded subdirectory in place: text files
   * like written artifacts, and binary files (agents' SQLite session stores)
   * by masking each secret with the same number of bytes. A SQLite database
   * is checkpointed first, so its write-ahead log is folded into the main
   * file: masking bytes inside a WAL would break its frame checksums and lose
   * committed records. A SQLite WAL (recognised by its header) that still
   * holds a secret afterwards is deleted; other files are masked in place.
   * Links are never followed: the sandbox wrote them, and a link resolving
   * outside the download (`../../.env`) would otherwise rewrite host files.
   */
  redactTree(subdir: string): void {
    const files: string[] = [];
    const walk = (dir: string) => {
      for (const name of readdirSync(dir)) {
        const file = join(dir, name);
        const stat = lstatSync(file);
        if (stat.isDirectory()) walk(file);
        else if (stat.isFile() && stat.size < 50 * 1024 * 1024) files.push(file);
      }
    };
    try {
      walk(join(this.path, subdir));
    } catch {
      return; // nothing downloaded
    }
    for (const file of files) if (existsSync(`${file}-wal`) && isSqlite(file)) checkpoint(file);
    for (const file of files) {
      if (!existsSync(file)) continue;
      const buf = readFileSync(file);
      if (buf.includes(0)) {
        const bytes = buf.toString('latin1');
        const clean = maskSameLength(bytes);
        if (clean === bytes) continue;
        if (isSqliteWal(buf)) rmSync(file);
        else writeFileSync(file, Buffer.from(clean, 'latin1'));
        continue;
      }
      const text = buf.toString('utf8');
      const clean = redact(text);
      if (clean !== text) writeFileSync(file, clean);
    }
  }
}

const SQLITE_HEADER = Buffer.from('SQLite format 3\0', 'latin1');

function isSqlite(file: string): boolean {
  return readFileSync(file).subarray(0, SQLITE_HEADER.length).equals(SQLITE_HEADER);
}

/** A SQLite write-ahead log, by its header's magic number. */
function isSqliteWal(buf: Buffer): boolean {
  return buf.length >= 32 && (buf.readUInt32BE(0) === 0x377f0682 || buf.readUInt32BE(0) === 0x377f0683);
}

/** Fold a copied database's write-ahead log into the main file. */
function checkpoint(file: string): void {
  try {
    const db = new DatabaseSync(file);
    try {
      db.exec('PRAGMA wal_checkpoint(TRUNCATE)');
    } finally {
      db.close();
    }
  } catch {
    // Unreadable: the WAL is masked or deleted below like any other file.
  }
}

const s = (ms: number | undefined) => (ms === undefined ? '-' : `${(ms / 1000).toFixed(1)}s`);
const n = (v: number) => v.toLocaleString('en-US');

export function formatSummary(report: RunReport): string {
  const { timing, tokens, cost, tools, verification, friction } = report;
  const m = timing.milestones;
  const failedChecks = verification.checks.filter((c) => !c.passed);
  const lines = [
    `=== ${report.task.name} · ${report.config.harness} · ${report.config.model} ===`,
    `  status        ${report.status}${report.error ? ` — ${report.error}` : ''}`,
    `  verification  ${verification.passed ? 'PASS' : 'FAIL'} (${verification.checks.filter((c) => c.passed).length}/${verification.checks.length} checks, score ${verification.score.toFixed(2)})`,
    ...failedChecks.map((c) => `    ✗ ${c.id}: ${c.detail}`),
    `  time          total ${s(timing.totalMs)} · setup ${s(timing.setupMs)} · agent ${s(timing.agentMs)} (model ${s(timing.modelMs)}, tools ${s(timing.toolMs)}) · verify ${s(timing.verifyMs)}`,
    `  ttft          first ${s(timing.ttftMs.first)} · median ${s(timing.ttftMs.median)} · max ${s(timing.ttftMs.max)}`,
    `  milestones    a4 ${s(m.firstA4CommandMs)} · doctor ok ${s(m.doctorOkMs)} · install ${s(m.firstDependencyInstallMs)} · program ran ${s(m.firstProgramRunMs)}`,
    `  tokens        in ${n(tokens.inputTokens)} (cache read ${n(tokens.cacheReadTokens)}, write ${n(tokens.cacheWriteTokens)}) · out ${n(tokens.outputTokens)} (reasoning ${n(tokens.reasoningTokens)})`,
    `  context       peak ${n(tokens.peakContextTokens)}${tokens.peakContextFraction !== undefined ? ` (${(tokens.peakContextFraction * 100).toFixed(1)}%)` : ''} · compactions ${tokens.compactions} · steps ${report.steps}`,
    `  cost          model $${cost.modelUsd.toFixed(4)} (${cost.source})${cost.harnessReportedUsd !== undefined ? ` · harness says $${cost.harnessReportedUsd.toFixed(4)}` : ''} · sandbox ≤ $${cost.sandboxUsdUpperBound.toFixed(4)}`,
    `  tools         ${tools.total} calls, ${tools.failed} failed · ${JSON.stringify(tools.byCategory)}`,
    `  a4            ${JSON.stringify(tools.a4Commands)} · failures ${tools.a4Failures} · help ${tools.helpLookups}`,
    `  mcp           ${JSON.stringify(tools.mcpCalls)} · skills read ${tools.skillReads} · docs ${tools.docsLookups} · direct API ${tools.directApiCalls}`,
    `  friction      ${JSON.stringify(friction.counts)}`,
  ];
  if (report.harness.apiRetries) {
    lines.push(`  api retries   ${report.harness.apiRetries} (${report.harness.rateLimitRetries} rate-limited) — timings include backoff`);
  }
  if (report.workspace) {
    const w = report.workspace;
    lines.push(
      `  workspace     +${w.created.length} ~${w.modified.length} -${w.deleted.length} files · lines app ${w.lines.app}, sdk ${w.lines.sdk}, arete-config ${w.lines['arete-config']}, config ${w.lines.config}`,
    );
  }
  if (report.arete.usageDelta) lines.push(`  arete usage   ${JSON.stringify(report.arete.usageDelta)} (${report.arete.usageAttribution})`);
  return lines.join('\n');
}
