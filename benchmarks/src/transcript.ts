import type { ClassifiedCall } from './classify.js';
import { preview, summarizeInput } from './recorder.js';
import type { Transcript } from './types.js';

const t = (ms: number) => `${(ms / 1000).toFixed(1)}s`;

function fence(text: string, lang = ''): string {
  const ticks = text.includes('```') ? '````' : '```';
  return `${ticks}${lang}\n${text}\n${ticks}`;
}

/**
 * Human-readable session history: every turn, step, message, reasoning
 * summary and tool call with its input, output, timing and failure flag.
 */
export function renderTranscript(
  transcript: Transcript,
  calls: ClassifiedCall[],
  opts: { outputChars?: number } = {},
): string {
  const outputChars = opts.outputChars ?? 3000;
  const byId = new Map(calls.map((c) => [c.record.id, c]));
  const out: string[] = [];
  for (const turn of transcript.turns) {
    out.push(`## Turn ${turn.turn + 1}${turn.freshSession ? ' (fresh session — host restarted)' : ''} · ${t(turn.startMs)}`);
    out.push('', '**User:**', '', turn.prompt, '');
    for (const step of transcript.steps.filter((s) => s.turn === turn.turn)) {
      const dur = step.endMs !== undefined ? t(step.endMs - step.startMs) : '?';
      const ttft = step.firstOutputMs !== undefined ? `, ttft ${t(step.firstOutputMs - step.readyMs)}` : '';
      const usage = step.usage
        ? `, in ${step.usage.inputTokens} (cached ${step.usage.cacheReadTokens}) / out ${step.usage.outputTokens}`
        : '';
      out.push(`### Step ${step.step + 1} · ${t(step.startMs)} · ${dur}${ttft}${usage}`, '');
      if (step.reasoning.trim()) {
        out.push('<details><summary>reasoning</summary>', '', preview(step.reasoning.trim(), 4000), '', '</details>', '');
      }
      if (step.text.trim()) out.push(step.text.trim(), '');
      for (const id of step.toolCallIds) {
        const call = byId.get(id);
        if (!call) continue;
        const r = call.record;
        const dur = r.endMs !== undefined ? t(r.endMs - r.startMs) : 'no result';
        const flags = [call.failed ? '❌ failed' : '', call.exitCode !== undefined ? `exit ${call.exitCode}` : '']
          .filter(Boolean)
          .join(', ');
        out.push(`**🔧 ${r.name}** · ${t(r.startMs)} · ${dur}${flags ? ` · ${flags}` : ''}`, '');
        const input = call.command ?? (typeof r.input === 'string' ? r.input : JSON.stringify(r.input, null, 2));
        out.push(fence(preview(input ?? summarizeInput(r.input), 4000), call.command ? 'bash' : ''), '');
        if (call.outputText.trim()) out.push(fence(preview(call.outputText.trim(), outputChars)), '');
      }
    }
    if (turn.error) out.push(`> **Turn error:** ${turn.error}`, '');
    if (turn.unfinished) out.push('> Turn ended waiting for host input.', '');
  }
  return out.join('\n');
}
