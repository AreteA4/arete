import { existsSync, readdirSync, readFileSync, statSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { parseArgs } from 'node:util';
import { classify } from '../classify.js';
import { resultsDir } from '../env.js';
import { detectFriction } from '../friction.js';
import { summarizeTiming, summarizeTokens, summarizeTools } from '../metrics.js';
import { backfillCodexUsage } from '../native-usage.js';
import { gatewayModelId, loadPricing, modelCost } from '../pricing.js';
import { normalizeUsage } from '../recorder.js';
import { formatSummary, redact } from '../report.js';
import { renderTranscript } from '../transcript.js';
import type { RunReport, Transcript } from '../types.js';

/** Older transcripts lack `readyMs`; rebuild it from turn starts and tool results. */
function normalize(transcript: Transcript): Transcript {
  // Early recordings timed from run start; shift them to agent start.
  const offset = transcript.turns[0]?.startMs ?? 0;
  if (offset > 0) {
    for (const turn of transcript.turns) {
      turn.startMs -= offset;
      if (turn.endMs !== undefined) turn.endMs -= offset;
    }
    for (const step of transcript.steps) {
      step.startMs -= offset;
      if (step.readyMs !== undefined) step.readyMs -= offset;
      if (step.firstOutputMs !== undefined) step.firstOutputMs -= offset;
      if (step.endMs !== undefined) step.endMs -= offset;
    }
    for (const call of transcript.toolCalls) {
      call.startMs -= offset;
      if (call.endMs !== undefined) call.endMs -= offset;
    }
    for (const error of transcript.errors) error.atMs -= offset;
  }
  for (const turn of transcript.turns) {
    if (turn.totalUsage) turn.totalUsage = normalizeUsage(turn.totalUsage);
  }
  for (const step of transcript.steps) {
    if (step.usage) step.usage = normalizeUsage(step.usage);
  }
  for (const step of transcript.steps) {
    if (step.readyMs !== undefined) continue;
    const turn = transcript.turns.find((t) => t.turn === step.turn);
    const boundaries = [
      turn?.startMs ?? 0,
      ...transcript.steps.filter((s) => s.turn === step.turn && s.endMs !== undefined && s.endMs <= step.startMs).map((s) => s.endMs!),
      ...transcript.toolCalls.filter((t) => t.turn === step.turn && t.endMs !== undefined && t.endMs <= step.startMs).map((t) => t.endMs!),
    ];
    step.readyMs = Math.max(...boundaries);
  }
  return transcript;
}

/**
 * Recompute every transcript-derived field of existing runs (tool summary,
 * friction, timing, tokens, cost) after the classifier or metrics change.
 * Verification needs the sandbox and is left as recorded.
 */
async function main(): Promise<void> {
  const { positionals } = parseArgs({ args: process.argv.slice(2), allowPositionals: true });
  const root = positionals[0] ?? join(resultsDir(), 'runs');
  const pricing = await loadPricing();
  const walk = (dir: string): string[] =>
    existsSync(join(dir, 'report.json'))
      ? [dir]
      : readdirSync(dir).flatMap((name) => (statSync(join(dir, name)).isDirectory() ? walk(join(dir, name)) : []));

  for (const dir of existsSync(root) ? walk(root) : []) {
    if (!existsSync(join(dir, 'transcript.json'))) continue;
    const report = JSON.parse(readFileSync(join(dir, 'report.json'), 'utf8')) as RunReport;
    const transcript = normalize(JSON.parse(readFileSync(join(dir, 'transcript.json'), 'utf8')) as Transcript);
    const codex = report.config.harness === 'codex' ? backfillCodexUsage(transcript, join(dir, 'native')) : undefined;
    writeFileSync(join(dir, 'transcript.json'), redact(`${JSON.stringify(transcript, null, 2)}\n`));
    const calls = transcript.toolCalls.map(classify);
    const price = pricing.get(gatewayModelId(report.config.model, report.config.harness));
    const tokens = summarizeTokens(transcript, codex?.contextWindow ?? price?.contextWindow);
    const cost = modelCost(price, tokens);
    const updated: RunReport = {
      ...report,
      harness: report.harness ?? { apiRetries: 0, rateLimitRetries: 0 },
      timing: summarizeTiming(report.timing, transcript, calls),
      tokens,
      cost: { ...report.cost, modelUsd: cost ?? 0, source: cost === undefined ? 'unpriced' : 'gateway-pricing' },
      tools: summarizeTools(calls),
      steps: transcript.steps.length,
      friction: detectFriction(transcript, calls),
    };
    writeFileSync(join(dir, 'report.json'), redact(`${JSON.stringify(updated, null, 2)}\n`));
    writeFileSync(join(dir, 'transcript.md'), redact(`# ${report.runId}\n\n${renderTranscript(transcript, calls)}\n`));
    writeFileSync(join(dir, 'summary.txt'), redact(`${formatSummary(updated)}\n`));
    process.stdout.write(`rescored ${report.runId}\n`);
  }
}

main().catch((err) => {
  process.stderr.write(`Fatal: ${err instanceof Error ? err.message : String(err)}\n`);
  process.exit(1);
});
