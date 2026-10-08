import type { ClassifiedCall } from './classify.js';
import type {
  Milestones,
  PhaseTimings,
  TimingSummary,
  TokenSummary,
  ToolSummary,
  Transcript,
  UsageRecord,
} from './types.js';

const ZERO: UsageRecord = {
  inputTokens: 0,
  noCacheTokens: 0,
  cacheReadTokens: 0,
  cacheWriteTokens: 0,
  outputTokens: 0,
  reasoningTokens: 0,
};

function add(a: UsageRecord, b: UsageRecord | undefined): UsageRecord {
  if (!b) return a;
  return {
    inputTokens: a.inputTokens + b.inputTokens,
    noCacheTokens: a.noCacheTokens + b.noCacheTokens,
    cacheReadTokens: a.cacheReadTokens + b.cacheReadTokens,
    cacheWriteTokens: a.cacheWriteTokens + b.cacheWriteTokens,
    outputTokens: a.outputTokens + b.outputTokens,
    reasoningTokens: a.reasoningTokens + b.reasoningTokens,
  };
}

function median(values: number[]): number | undefined {
  if (!values.length) return undefined;
  const sorted = [...values].sort((a, b) => a - b);
  const mid = Math.floor(sorted.length / 2);
  return sorted.length % 2 ? sorted[mid]! : Math.round((sorted[mid - 1]! + sorted[mid]!) / 2);
}

/**
 * Turn totals come from the `finish` part when it carries tokens (Codex only
 * reports usage per turn); otherwise from the sum of the turn's steps.
 */
export function totalUsage(transcript: Transcript): UsageRecord {
  let total = ZERO;
  for (const turn of transcript.turns) {
    const stepSum = transcript.steps
      .filter((s) => s.turn === turn.turn)
      .reduce((acc, s) => add(acc, s.usage), ZERO);
    const reported = turn.totalUsage;
    const useReported =
      reported && reported.inputTokens + reported.outputTokens >= stepSum.inputTokens + stepSum.outputTokens;
    total = add(total, useReported ? reported : stepSum);
  }
  return total;
}

export function summarizeTokens(transcript: Transcript, contextWindow?: number): TokenSummary {
  const usage = totalUsage(transcript);
  const peak = Math.max(0, ...transcript.steps.map((s) => s.usage?.inputTokens ?? 0));
  return {
    ...usage,
    totalTokens: usage.inputTokens + usage.outputTokens,
    peakContextTokens: peak,
    ...(contextWindow ? { contextWindow, peakContextFraction: peak / contextWindow } : {}),
    compactions: transcript.toolCalls.filter((t) => t.name === 'compaction').length,
  };
}

export function summarizeTools(calls: ClassifiedCall[]): ToolSummary {
  const summary: ToolSummary = {
    total: calls.length,
    failed: 0,
    byName: {},
    byCategory: {},
    a4Commands: {},
    a4Failures: 0,
    mcpCalls: {},
    helpLookups: 0,
    skillReads: 0,
    docsLookups: 0,
    directApiCalls: 0,
    toolTimeMs: 0,
  };
  const bump = (map: Record<string, number>, key: string) => (map[key] = (map[key] ?? 0) + 1);
  for (const call of calls) {
    bump(summary.byName, call.record.name);
    bump(summary.byCategory, call.category);
    if (call.failed) summary.failed++;
    for (const a4 of call.a4) {
      bump(summary.a4Commands, a4.path);
      if (a4.help) summary.helpLookups++;
    }
    if (call.a4.length && call.failed) summary.a4Failures++;
    if (call.mcp) bump(summary.mcpCalls, `${call.mcp.server}/${call.mcp.tool}`);
    if (call.skillRead) summary.skillReads++;
    if (call.docsLookup) summary.docsLookups++;
    if (call.directApi) summary.directApiCalls++;
    if (call.record.endMs !== undefined) summary.toolTimeMs += call.record.endMs - call.record.startMs;
  }
  return summary;
}

function doctorStatus(call: ClassifiedCall): string | undefined {
  return /^\s{0,2}"status":\s*"(\w+)"/m.exec(call.outputText)?.[1];
}

export function findMilestones(calls: ClassifiedCall[]): Milestones {
  const first = (pred: (c: ClassifiedCall) => boolean) => calls.find(pred)?.record.startMs;
  return {
    firstA4CommandMs: first((c) => c.a4.length > 0),
    a4InstalledMs: first(
      (c) => !c.failed && !!c.command && /install\.sh|@usearete\/a4(@[\w.-]+)?\s+install/.test(c.command),
    ),
    doctorOkMs: first((c) => c.a4.some((a) => a.path === 'doctor') && doctorStatus(c) === 'ok'),
    firstDependencyInstallMs: first(
      (c) => !c.failed && c.a4.some((a) => a.path.startsWith('install') && !a.help),
    ),
    firstProgramRunMs: first((c) => c.programRun && !c.failed),
  };
}

export function summarizeTiming(
  phases: PhaseTimings,
  transcript: Transcript,
  calls: ClassifiedCall[],
): TimingSummary {
  const ttfts = transcript.steps
    .filter((s) => s.firstOutputMs !== undefined)
    .map((s) => s.firstOutputMs! - s.readyMs);
  const toolMs = calls.reduce(
    (sum, c) => sum + (c.record.endMs !== undefined ? c.record.endMs - c.record.startMs : 0),
    0,
  );
  return {
    ...phases,
    turnsMs: transcript.turns.map((t) => (t.endMs ?? t.startMs) - t.startMs),
    modelMs: Math.max(0, phases.agentMs - toolMs),
    toolMs,
    ttftMs: { first: ttfts[0], median: median(ttfts), max: ttfts.length ? Math.max(...ttfts) : undefined },
    milestones: findMilestones(calls),
  };
}
