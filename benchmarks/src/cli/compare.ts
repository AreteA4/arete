import { join } from 'node:path';
import { parseArgs } from 'node:util';
import { resultsDir } from '../env.js';
import { loadReports, quantile, runLabel } from '../history.js';
import type { RunReport } from '../types.js';

const fmt = {
  s: (ms?: number) => (ms === undefined ? '–' : `${(ms / 1000).toFixed(0)}s`),
  k: (v?: number) => (v === undefined ? '–' : v >= 1000 ? `${(v / 1000).toFixed(0)}k` : `${Math.round(v)}`),
  usd: (v?: number) => (v === undefined ? '–' : `$${v.toFixed(3)}`),
  n: (v?: number) => (v === undefined ? '–' : v.toFixed(1)),
};

/**
 * Markdown comparison table grouped by task × grading × fresh account × harness × model,
 * with medians (p90 for duration) across repetitions. Runs graded under
 * different rules never share a row, since their pass flags mean different
 * things.
 */
export function compareTable(reports: RunReport[]): string {
  const groups = new Map<string, RunReport[]>();
  for (const r of reports) {
    const key = `${runLabel(r)}\u0000${r.config.harness}\u0000${r.config.model}`;
    groups.set(key, [...(groups.get(key) ?? []), r]);
  }
  const header =
    '| task | harness | model | runs | errors | pass | time p50 | time p90 | doctor ok p50 | in tok p50 | out tok p50 | peak ctx p50 | cost p50 | tools p50 | failed p50 | a4 cmds p50 | help p50 | mcp p50 |';
  const rows = [header, header.replace(/[^|]+/g, '---')];
  for (const [key, all] of [...groups.entries()].sort()) {
    const [task, harness, model] = key.split('\u0000');
    // Setup and infrastructure failures say nothing about the agent; keep them out of the stats.
    const group = all.filter((r) => r.status !== 'infra-error' && r.status !== 'setup-error');
    const errors = all.length - group.length;
    const med = (f: (r: RunReport) => number | undefined) =>
      quantile(group.map(f).filter((v): v is number => v !== undefined), 0.5);
    const pass = group.filter((r) => r.verification.passed).length;
    rows.push(
      `| ${task} | ${harness} | ${model} | ${all.length} | ${errors} | ${group.length ? `${pass}/${group.length}` : '–'} | ` +
        `${fmt.s(med((r) => r.timing.totalMs))} | ${fmt.s(quantile(group.map((r) => r.timing.totalMs), 0.9))} | ` +
        `${fmt.s(med((r) => r.timing.milestones.doctorOkMs))} | ${fmt.k(med((r) => r.tokens.inputTokens))} | ` +
        `${fmt.k(med((r) => r.tokens.outputTokens))} | ${fmt.k(med((r) => r.tokens.peakContextTokens || undefined))} | ` +
        `${fmt.usd(med((r) => r.cost.modelUsd))} | ${fmt.n(med((r) => r.tools.total))} | ` +
        `${fmt.n(med((r) => r.tools.failed))} | ${fmt.n(med((r) => Object.values(r.tools.a4Commands).reduce((a, b) => a + b, 0)))} | ` +
        `${fmt.n(med((r) => r.tools.helpLookups))} | ${fmt.n(med((r) => Object.values(r.tools.mcpCalls).reduce((a, b) => a + b, 0)))} |`,
    );
  }
  return rows.join('\n');
}

/** Friction totals across runs, most frequent first. */
export function frictionTable(all: RunReport[]): string {
  const reports = all.filter((r) => r.status !== 'infra-error' && r.status !== 'setup-error');
  const totals = new Map<string, number>();
  for (const r of reports) {
    for (const [kind, count] of Object.entries(r.friction.counts)) {
      totals.set(kind, (totals.get(kind) ?? 0) + (count ?? 0));
    }
  }
  const rows = ['| friction | events | runs affected |', '| --- | --- | --- |'];
  for (const [kind, total] of [...totals.entries()].sort((a, b) => b[1] - a[1])) {
    const runs = reports.filter((r) => (r.friction.counts as Record<string, number>)[kind]).length;
    rows.push(`| ${kind} | ${total} | ${runs}/${reports.length} |`);
  }
  return rows.join('\n');
}

function main(): void {
  const { values, positionals } = parseArgs({
    args: process.argv.slice(2),
    allowPositionals: true,
    options: { since: { type: 'string' }, task: { type: 'string' } },
  });
  const root = positionals[0] ?? join(resultsDir(), 'runs');
  let reports = loadReports(root);
  if (values.since) reports = reports.filter((r) => r.startedAt >= values.since!);
  if (values.task) reports = reports.filter((r) => r.task.name === values.task);
  if (!reports.length) {
    process.stdout.write(`No reports under ${root}\n`);
    return;
  }
  process.stdout.write(`${compareTable(reports)}\n\n${frictionTable(reports)}\n`);
}

if (import.meta.url === `file://${process.argv[1]}`) main();
