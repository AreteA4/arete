import { createAnthropic } from '@ai-sdk/anthropic';
import { generateText, Output, type LanguageModel } from 'ai';
import { existsSync, mkdirSync, readdirSync, readFileSync, statSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { parseArgs } from 'node:util';
import { z } from 'zod';
import { classify } from '../classify.js';
import { resultsDir } from '../env.js';
import { redact } from '../report.js';
import { renderTranscript } from '../transcript.js';
import type { RunReport, Transcript } from '../types.js';

const CATEGORIES = [
  'docs',
  'skill',
  'cli-ux',
  'cli-bug',
  'sdk',
  'catalog',
  'mcp',
  'agent-behavior',
  'harness',
  'task',
] as const;

const findingSchema = z.object({
  category: z.enum(CATEGORIES),
  severity: z.enum(['high', 'medium', 'low']),
  title: z.string().describe('One line naming the problem, phrased so the same issue gets the same title across runs'),
  evidence: z.string().describe('Quotes from the transcript with step references, e.g. "Turn 1 step 4: …"'),
  suggestion: z.string().describe('Concrete change to Arete docs, skills, CLI, SDK or catalog that would have prevented it'),
  wastedSeconds: z.number().optional().describe('Approximate agent time lost to this issue'),
});

const reviewSchema = z.object({
  outcome: z.enum(['success', 'partial', 'failure']),
  summary: z.string().describe('Three to five sentences on what the agent did and how it went'),
  keyMoments: z
    .array(z.object({ atSeconds: z.number(), what: z.string() }))
    .describe('The turning points of the session, at most 12'),
  findings: z.array(findingSchema).describe('Improvement opportunities for Arete, most important first'),
});

type Review = z.infer<typeof reviewSchema>;

const REVIEW_INSTRUCTIONS = `You review recorded sessions of AI coding agents using Arete, an agent-first Solana data and transaction toolkit driven through the \`a4\` CLI, its skills, MCP servers and generated SDKs. The agent ran unattended in a fresh sandbox.

Your job is to find what Arete should change so the next agent succeeds faster and with fewer tokens. Look for:
- docs or skill guidance that was missing, wrong, ambiguous or ignored;
- CLI commands, flags, errors or output that confused the agent or forced retries;
- generated SDK names or types the agent guessed wrong;
- catalog entries that were hard to find or misleading;
- MCP tools that failed, were unused, or returned unhelpful output;
- agent behaviour that wasted time (loops, guessing endpoints, editing generated files) and what guidance would prevent it;
- harness or task problems that make the measurement unfair (category "harness" or "task").

Ground every finding in the transcript with quotes and step references. Do not report things that went fine. Prefer a few precise findings over many vague ones.`;

function modelFor(id: string, auth: string): LanguageModel {
  if (auth === 'direct') {
    return createAnthropic()(id.replace(/^anthropic\//, '').replace(/(\d)\.(\d)/g, '$1-$2'));
  }
  return id;
}

function findRunDirs(root: string): string[] {
  const dirs: string[] = [];
  const walk = (dir: string) => {
    if (existsSync(join(dir, 'report.json'))) {
      dirs.push(dir);
      return;
    }
    for (const name of readdirSync(dir)) {
      const path = join(dir, name);
      if (statSync(path).isDirectory()) walk(path);
    }
  };
  if (existsSync(root)) walk(root);
  return dirs.sort();
}

function reviewInput(runDir: string): string {
  const report = JSON.parse(readFileSync(join(runDir, 'report.json'), 'utf8')) as RunReport;
  const transcript = JSON.parse(readFileSync(join(runDir, 'transcript.json'), 'utf8')) as Transcript;
  const calls = transcript.toolCalls.map(classify);
  let rendered = renderTranscript(transcript, calls, { outputChars: 1200 });
  const LIMIT = 150_000;
  if (rendered.length > LIMIT) {
    rendered = `${rendered.slice(0, LIMIT / 2)}\n\n[… ${rendered.length - LIMIT} chars omitted …]\n\n${rendered.slice(-LIMIT / 2)}`;
  }
  const facts = {
    task: report.task,
    harness: report.config.harness,
    model: report.config.model,
    status: report.status,
    error: report.error,
    verification: report.verification,
    timing: report.timing,
    tokens: report.tokens,
    tools: report.tools,
    arete: report.arete,
  };
  return [
    '## Run facts',
    '```json',
    JSON.stringify(facts, null, 2),
    '```',
    '## Deterministic friction signals',
    '```json',
    JSON.stringify(report.friction.events.slice(0, 60), null, 2),
    '```',
    '## Transcript',
    rendered,
  ].join('\n');
}

function renderReview(review: Review, runDir: string): string {
  const lines = [
    `# Review: ${runDir.split('/').pop()}`,
    '',
    `**Outcome:** ${review.outcome}`,
    '',
    review.summary,
    '',
    '## Key moments',
    ...review.keyMoments.map((m) => `- ${m.atSeconds.toFixed(0)}s — ${m.what}`),
    '',
    '## Findings',
  ];
  for (const f of review.findings) {
    lines.push(
      '',
      `### [${f.severity}] ${f.category}: ${f.title}`,
      '',
      `**Evidence:** ${f.evidence}`,
      '',
      `**Suggestion:** ${f.suggestion}${f.wastedSeconds ? ` _(≈${Math.round(f.wastedSeconds)}s lost)_` : ''}`,
    );
  }
  return `${lines.join('\n')}\n`;
}

const aggregateSchema = z.object({
  overview: z.string(),
  improvements: z.array(
    z.object({
      title: z.string(),
      category: z.enum(CATEGORIES),
      priority: z.enum(['p0', 'p1', 'p2']),
      runsAffected: z.number(),
      harnesses: z.array(z.string()),
      evidence: z.string(),
      recommendation: z.string(),
    }),
  ),
});

/** Default aggregation window; results accumulate weekly, so all-time would grow without bound. */
const AGGREGATE_WINDOW_DAYS = 28;

async function aggregate(dirs: string[], since: string, model: LanguageModel, modelId: string, out: string): Promise<void> {
  const reviews = dirs
    .filter((d) => existsSync(join(d, 'review.json')))
    .map((d) => ({ d, report: JSON.parse(readFileSync(join(d, 'report.json'), 'utf8')) as RunReport }))
    .filter(({ report }) => report.startedAt >= since)
    .filter(({ report }) => report.status !== 'infra-error' && report.status !== 'setup-error')
    .map(({ d, report }) => {
      const { reviewer: _reviewer, ...review } = JSON.parse(readFileSync(join(d, 'review.json'), 'utf8')) as Review & { reviewer?: unknown };
      return { run: report.runId, task: report.task.name, harness: report.config.harness, model: report.config.model, passed: report.verification.passed, ...review };
    });
  if (!reviews.length) throw new Error(`No reviewed runs since ${since}; run \`npm run review\` first.`);
  const { output, usage } = await generateText({
    model,
    instructions:
      'You merge per-run reviews of agents using Arete into one ranked improvement list. Merge findings that describe the same root cause, count the runs they affect, and rank by impact on agent success, time and tokens. Be concrete.',
    prompt: JSON.stringify(reviews),
    output: Output.object({ schema: aggregateSchema }),
  });
  const lines = [`# Arete agent findings — ${new Date().toISOString().slice(0, 10)}`, '', `${reviews.length} reviewed runs since ${since}.`, '', output.overview, ''];
  for (const item of output.improvements) {
    lines.push(
      `## ${item.priority.toUpperCase()} · ${item.category} · ${item.title}`,
      '',
      `Runs affected: ${item.runsAffected} (${item.harnesses.join(', ')})`,
      '',
      `**Evidence:** ${item.evidence}`,
      '',
      `**Recommendation:** ${item.recommendation}`,
      '',
    );
  }
  mkdirSync(dirname(out), { recursive: true });
  writeFileSync(out, redact(lines.join('\n')));
  const reviewer = { model: modelId, inputTokens: usage.inputTokens ?? 0, outputTokens: usage.outputTokens ?? 0 };
  writeFileSync(out.replace(/\.md$/, '.json'), redact(JSON.stringify({ ...output, reviewer }, null, 2)));
  process.stdout.write(`wrote ${out}\n`);
}

async function main(): Promise<void> {
  const { values, positionals } = parseArgs({
    args: process.argv.slice(2),
    allowPositionals: true,
    options: {
      model: { type: 'string', default: 'anthropic/claude-sonnet-5.5' },
      'model-auth': { type: 'string', default: process.env.AI_GATEWAY_API_KEY || process.env.VERCEL_OIDC_TOKEN ? 'ai-gateway' : 'direct' },
      force: { type: 'boolean', default: false },
      aggregate: { type: 'boolean', default: false },
      since: { type: 'string' },
      concurrency: { type: 'string', default: '4' },
    },
  });
  const root = positionals[0] ?? join(resultsDir(), 'runs');
  const model = modelFor(values.model!, values['model-auth']!);
  const dirs = findRunDirs(root);

  if (values.aggregate) {
    const since = values.since ?? new Date(Date.now() - AGGREGATE_WINDOW_DAYS * 86_400_000).toISOString().slice(0, 10);
    await aggregate(dirs, since, model, values.model!, join(resultsDir(), 'reports', `${new Date().toISOString().slice(0, 10)}-findings.md`));
    return;
  }

  // Setup and infrastructure failures have no agent session worth reviewing.
  const pending = dirs.filter((dir) => {
    if (!values.force && existsSync(join(dir, 'review.json'))) return false;
    const { status } = JSON.parse(readFileSync(join(dir, 'report.json'), 'utf8')) as RunReport;
    return status !== 'infra-error' && status !== 'setup-error';
  });
  process.stdout.write(`${pending.length} run(s) to review\n`);
  const reviewOne = async (dir: string) => {
    const { output, usage } = await generateText({
      model,
      instructions: REVIEW_INSTRUCTIONS,
      prompt: reviewInput(dir),
      output: Output.object({ schema: reviewSchema }),
    });
    const reviewer = { model: values.model, inputTokens: usage.inputTokens ?? 0, outputTokens: usage.outputTokens ?? 0 };
    writeFileSync(join(dir, 'review.json'), redact(JSON.stringify({ ...output, reviewer }, null, 2)));
    writeFileSync(join(dir, 'review.md'), redact(renderReview(output, dir)));
    process.stdout.write(`  ${dir.split('/').pop()}: ${output.outcome}, ${output.findings.length} finding(s) · ${usage.inputTokens ?? 0} in / ${usage.outputTokens ?? 0} out tokens\n`);
  };
  let next = 0;
  let failed = 0;
  const worker = async () => {
    while (next < pending.length) {
      const dir = pending[next++]!;
      try {
        await reviewOne(dir);
      } catch (err) {
        failed++;
        process.stderr.write(`  review failed for ${dir}: ${err instanceof Error ? err.message : String(err)}\n`);
      }
    }
  };
  await Promise.all(Array.from({ length: Math.min(Number(values.concurrency), pending.length) }, worker));
  if (failed) process.exitCode = 1;

}

main().catch((err) => {
  process.stderr.write(`Fatal: ${err instanceof Error ? err.message : String(err)}\n`);
  process.exit(1);
});
