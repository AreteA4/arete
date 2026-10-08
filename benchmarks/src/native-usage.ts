import { existsSync, readdirSync, readFileSync, statSync } from 'node:fs';
import { join } from 'node:path';
import type { Transcript, UsageRecord } from './types.js';

interface CodexTokenUsage {
  input_tokens?: number;
  cached_input_tokens?: number;
  cache_write_input_tokens?: number;
  output_tokens?: number;
  reasoning_output_tokens?: number;
  total_tokens?: number;
}

interface CodexRollout {
  /** Usage of each model call, in order. */
  calls: UsageRecord[];
  contextWindow?: number;
}

function findRollouts(dir: string): string[] {
  if (!existsSync(dir)) return [];
  return readdirSync(dir)
    .flatMap((name) => {
      const path = join(dir, name);
      if (statSync(path).isDirectory()) return findRollouts(path);
      return /^rollout-.*\.jsonl$/.test(name) ? [path] : [];
    })
    .sort();
}

function toUsage(u: CodexTokenUsage): UsageRecord {
  const input = u.input_tokens ?? 0;
  const cacheRead = u.cached_input_tokens ?? 0;
  const cacheWrite = u.cache_write_input_tokens ?? 0;
  return {
    inputTokens: input,
    noCacheTokens: Math.max(0, input - cacheRead - cacheWrite),
    cacheReadTokens: cacheRead,
    cacheWriteTokens: cacheWrite,
    outputTokens: u.output_tokens ?? 0,
    reasoningTokens: u.reasoning_output_tokens ?? 0,
  };
}

/**
 * Per-call usage from Codex's own rollout logs. Codex emits a `token_count`
 * event after every model call; repeats (same running total) are skipped.
 */
export function readCodexRollouts(nativeDir: string): CodexRollout[] {
  return findRollouts(join(nativeDir, '.codex', 'sessions')).map((file) => {
    const rollout: CodexRollout = { calls: [] };
    let previousTotal: number | undefined;
    for (const line of readFileSync(file, 'utf8').split('\n')) {
      if (!line.includes('"token_count"')) continue;
      let event: { payload?: { type?: string; info?: { total_token_usage?: CodexTokenUsage; last_token_usage?: CodexTokenUsage; model_context_window?: number } | null } };
      try {
        event = JSON.parse(line);
      } catch {
        continue;
      }
      const info = event.payload?.type === 'token_count' ? event.payload.info : undefined;
      if (!info?.last_token_usage) continue;
      const total = info.total_token_usage?.total_tokens;
      if (total !== undefined && total === previousTotal) continue;
      previousTotal = total;
      rollout.calls.push(toUsage(info.last_token_usage));
      rollout.contextWindow = info.model_context_window ?? rollout.contextWindow;
    }
    return rollout;
  });
}

/**
 * The Codex harness reports every step's usage as zero (only turn totals are
 * real). Fill per-step usage from the rollout logs when each turn's model
 * calls line up one-to-one with its steps, and return Codex's own context
 * window, which is smaller than the gateway catalog's figure.
 */
export function backfillCodexUsage(transcript: Transcript, nativeDir: string): { contextWindow?: number; filled: boolean } {
  const rollouts = readCodexRollouts(nativeDir);
  const contextWindow = rollouts.find((r) => r.contextWindow)?.contextWindow;
  const stepsReported = transcript.steps.some((s) => (s.usage?.inputTokens ?? 0) + (s.usage?.outputTokens ?? 0) > 0);
  if (stepsReported || rollouts.length === 0) return { contextWindow, filled: false };

  // A fresh session per turn writes a new rollout file; otherwise one file holds every turn.
  const calls = rollouts.length === transcript.turns.length ? rollouts.map((r) => r.calls) : [rollouts.flatMap((r) => r.calls)];
  const steps = calls.length === 1 ? [transcript.steps] : transcript.turns.map((t) => transcript.steps.filter((s) => s.turn === t.turn));
  if (calls.some((c, i) => c.length !== steps[i]?.length)) return { contextWindow, filled: false };
  calls.forEach((turnCalls, i) => turnCalls.forEach((usage, j) => (steps[i]![j]!.usage = usage)));
  return { contextWindow, filled: true };
}
