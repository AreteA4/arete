import { mkdirSync, readdirSync, readFileSync, statSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { knownSecrets } from './env.js';
import type { RunReport } from './types.js';

const KEY_PATTERN = /\ba4_(?:ak|sk|pk)_[A-Za-z0-9]{16,}\b/g;
const BEARER_PATTERN = /\b(Bearer\s+)[A-Za-z0-9._~+/=-]{20,}/g;

export function redact(text: string, secrets: string[] = knownSecrets()): string {
  let out = text;
  for (const secret of secrets) out = out.split(secret).join('[REDACTED]');
  return out.replace(KEY_PATTERN, '[REDACTED_A4_KEY]').replace(BEARER_PATTERN, '$1[REDACTED]');
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

  /** Redact every text file under a downloaded subdirectory in place. */
  redactTree(subdir: string): void {
    const walk = (dir: string) => {
      for (const name of readdirSync(dir)) {
        const file = join(dir, name);
        const stat = statSync(file);
        if (stat.isDirectory()) walk(file);
        else if (stat.size < 50 * 1024 * 1024) {
          const buf = readFileSync(file);
          if (buf.includes(0)) continue; // binary
          const text = buf.toString('utf8');
          const clean = redact(text);
          if (clean !== text) writeFileSync(file, clean);
        }
      }
    };
    try {
      walk(join(this.path, subdir));
    } catch {
      // nothing downloaded
    }
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
