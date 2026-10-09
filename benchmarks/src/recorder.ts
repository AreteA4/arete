import type { LanguageModelUsage, TextStreamPart, ToolSet } from 'ai';
import { redact } from './report.js';
import type { StepRecord, ToolCallRecord, Transcript, TurnRecord, UsageRecord } from './types.js';

export interface RecordedEvent {
  /** ms since agent start, measured on the host when the part arrived. */
  t: number;
  turn: number;
  step?: number;
  type: string;
  [key: string]: unknown;
}

const PREVIEW_CHARS = 4000;

export function preview(value: unknown, max = PREVIEW_CHARS): string {
  const text = typeof value === 'string' ? value : JSON.stringify(value) ?? '';
  return text.length > max ? `${text.slice(0, max)}… [${text.length - max} more chars]` : text;
}

/**
 * Make `inputTokens` the full prompt size. Most harnesses report it that
 * way, but OpenCode reports only the uncached part, with cache reads and
 * writes on top. Idempotent.
 */
export function normalizeUsage(usage: UsageRecord): UsageRecord {
  const cached = usage.cacheReadTokens + usage.cacheWriteTokens;
  if (cached <= usage.inputTokens) {
    return { ...usage, noCacheTokens: usage.noCacheTokens || usage.inputTokens - cached };
  }
  return { ...usage, inputTokens: usage.inputTokens + cached, noCacheTokens: usage.inputTokens };
}

export function toUsage(usage: LanguageModelUsage | undefined): UsageRecord | undefined {
  if (!usage) return undefined;
  return normalizeUsage({
    inputTokens: usage.inputTokens ?? 0,
    noCacheTokens: usage.inputTokenDetails?.noCacheTokens ?? 0,
    cacheReadTokens: usage.inputTokenDetails?.cacheReadTokens ?? 0,
    cacheWriteTokens: usage.inputTokenDetails?.cacheWriteTokens ?? 0,
    outputTokens: usage.outputTokens ?? 0,
    reasoningTokens: usage.outputTokenDetails?.reasoningTokens ?? 0,
  });
}

/**
 * Consumes harness stream parts and turns them into a timestamped event log
 * plus a structured transcript (turns → steps → text, reasoning, tool calls
 * with full inputs and outputs). Timestamps are taken on the host because
 * the harness does not report step or built-in tool timings itself.
 */
export class Recorder {
  readonly transcript: Transcript = { turns: [], steps: [], toolCalls: [], errors: [] };
  readonly events: RecordedEvent[] = [];
  /** Re-based to the first turn, so every timestamp is ms since agent start. */
  private t0 = performance.now();
  private started = false;
  /** Last moment the model was handed control (turn start, step end, tool result). */
  private boundaryMs = 0;
  private currentTurn?: TurnRecord;
  private currentStep?: StepRecord;
  private stepInTurn = 0;
  private readonly openBlocks = new Map<string, { kind: 'text' | 'reasoning'; startMs: number; text: string }>();
  private readonly toolsById = new Map<string, ToolCallRecord>();

  /**
   * `onProgress` lines go to the console, which is a public log in CI. They
   * are redacted before truncation, so a key cut short cannot slip through.
   */
  constructor(private readonly onProgress?: (line: string) => void) {}

  now(): number {
    return Math.round(performance.now() - this.t0);
  }

  beginTurn(turn: number, prompt: string, freshSession: boolean): void {
    if (!this.started) {
      this.started = true;
      this.t0 = performance.now();
    }
    this.boundaryMs = this.now();
    this.currentTurn = {
      turn,
      prompt,
      freshSession,
      startMs: this.now(),
      unfinished: false,
      text: '',
    };
    this.stepInTurn = 0;
    this.currentStep = undefined;
    this.transcript.turns.push(this.currentTurn);
    this.push({ type: 'turn-start', prompt, freshSession });
  }

  endTurn(fields: Partial<Pick<TurnRecord, 'unfinished' | 'error' | 'providerMetadata'>>): void {
    const turn = this.currentTurn;
    if (!turn) return;
    Object.assign(turn, fields);
    turn.endMs = this.now();
    if (fields.error) {
      this.transcript.errors.push({ atMs: turn.endMs, turn: turn.turn, message: fields.error });
    }
    this.push({ type: 'turn-end', ...fields });
  }

  async consume(stream: AsyncIterable<TextStreamPart<ToolSet>>): Promise<void> {
    for await (const part of stream) this.handle(part);
  }

  private push(event: { type: string; [key: string]: unknown }): void {
    this.events.push({
      t: this.now(),
      turn: this.currentTurn?.turn ?? 0,
      ...(this.currentStep ? { step: this.currentStep.step } : {}),
      ...event,
    });
  }

  private ensureStep(): StepRecord {
    if (this.currentStep && this.currentStep.endMs === undefined) return this.currentStep;
    const step: StepRecord = {
      turn: this.currentTurn?.turn ?? 0,
      step: this.stepInTurn++,
      startMs: this.now(),
      readyMs: this.boundaryMs,
      text: '',
      reasoning: '',
      toolCallIds: [],
    };
    this.currentStep = step;
    this.transcript.steps.push(step);
    return step;
  }

  private markOutput(): void {
    const step = this.ensureStep();
    step.firstOutputMs ??= this.now();
  }

  private toolRecord(id: string, name: string): ToolCallRecord {
    let record = this.toolsById.get(id);
    if (!record) {
      const step = this.ensureStep();
      record = {
        id,
        turn: step.turn,
        step: step.step,
        name,
        input: undefined,
        isError: false,
        providerExecuted: false,
        startMs: this.now(),
      };
      step.toolCallIds.push(id);
      this.toolsById.set(id, record);
      this.transcript.toolCalls.push(record);
    }
    return record;
  }

  private handle(part: TextStreamPart<ToolSet>): void {
    switch (part.type) {
      case 'start-step': {
        if (this.currentStep && this.currentStep.endMs === undefined) {
          this.currentStep.endMs = this.now();
        }
        this.currentStep = undefined;
        const step = this.ensureStep();
        this.push({ type: 'step-start', step: step.step });
        return;
      }
      case 'text-start':
      case 'reasoning-start': {
        this.markOutput();
        const kind = part.type === 'text-start' ? 'text' : 'reasoning';
        this.openBlocks.set(`${kind}:${part.id}`, { kind, startMs: this.now(), text: '' });
        return;
      }
      case 'text-delta':
      case 'reasoning-delta': {
        this.markOutput();
        const kind = part.type === 'text-delta' ? 'text' : 'reasoning';
        const key = `${kind}:${part.id}`;
        let block = this.openBlocks.get(key);
        if (!block) {
          block = { kind, startMs: this.now(), text: '' };
          this.openBlocks.set(key, block);
        }
        block.text += part.text;
        const step = this.ensureStep();
        if (kind === 'text') {
          step.text += part.text;
          if (this.currentTurn) this.currentTurn.text += part.text;
        } else {
          step.reasoning += part.text;
        }
        return;
      }
      case 'text-end':
      case 'reasoning-end': {
        const kind = part.type === 'text-end' ? 'text' : 'reasoning';
        const key = `${kind}:${part.id}`;
        const block = this.openBlocks.get(key);
        this.openBlocks.delete(key);
        if (block && block.text) {
          this.push({ type: kind, startMs: block.startMs, text: block.text });
          if (kind === 'text') this.onProgress?.(`text: ${preview(redact(block.text.trim()), 160)}`);
        }
        return;
      }
      case 'tool-input-start':
      case 'tool-input-delta':
        this.markOutput();
        return;
      case 'tool-call': {
        const record = this.toolRecord(part.toolCallId, part.toolName);
        record.input = part.input;
        record.providerExecuted = Boolean(part.providerExecuted);
        this.push({ type: 'tool-call', toolCallId: part.toolCallId, toolName: part.toolName, input: part.input });
        this.onProgress?.(`${part.toolName}: ${preview(redact(summarizeInput(part.input)), 160)}`);
        return;
      }
      case 'tool-result':
      case 'tool-error': {
        const record = this.toolRecord(part.toolCallId, part.toolName);
        record.endMs = this.now();
        this.boundaryMs = record.endMs;
        record.isError = part.type === 'tool-error';
        record.output = part.type === 'tool-error' ? errorText(part.error) : part.output;
        this.push({
          type: part.type,
          toolCallId: part.toolCallId,
          toolName: part.toolName,
          durationMs: record.endMs - record.startMs,
          output: preview(record.output),
        });
        return;
      }
      case 'tool-output-denied': {
        const record = this.toolRecord(part.toolCallId, part.toolName);
        record.endMs = this.now();
        record.isError = true;
        record.output = 'denied';
        this.push({ type: 'tool-denied', toolCallId: part.toolCallId, toolName: part.toolName });
        return;
      }
      case 'finish-step': {
        const step = this.ensureStep();
        step.endMs = this.now();
        this.boundaryMs = step.endMs;
        step.finishReason = part.finishReason;
        step.usage = toUsage(part.usage);
        this.push({ type: 'step-finish', finishReason: part.finishReason, usage: step.usage });
        return;
      }
      case 'finish': {
        if (this.currentTurn) {
          this.currentTurn.finishReason = part.finishReason;
          this.currentTurn.totalUsage = toUsage(part.totalUsage);
        }
        this.push({ type: 'finish', finishReason: part.finishReason, totalUsage: toUsage(part.totalUsage) });
        return;
      }
      case 'error': {
        const message = errorText(part.error);
        this.transcript.errors.push({ atMs: this.now(), turn: this.currentTurn?.turn ?? 0, message });
        this.push({ type: 'error', message });
        this.onProgress?.(`error: ${preview(redact(message), 200)}`);
        return;
      }
      case 'abort':
        this.push({ type: 'abort' });
        return;
      case 'raw':
        this.push({ type: 'raw', value: preview(part.rawValue, 8000) });
        return;
      default:
        this.push({ type: part.type });
    }
  }
}

/**
 * Who to blame for a turn that ended on a stream `error` part. If the model
 * never produced anything (no tokens, no tool calls, no text or reasoning)
 * the failure came from configuration or the provider, not the agent. Some
 * harnesses emit `start-step` before the request fails, so an empty step on
 * its own does not count as model work.
 */
export function turnErrorStatus(transcript: Transcript, turn: number): 'agent-error' | 'infra-error' {
  const steps = transcript.steps.filter((s) => s.turn === turn);
  const usages = [
    ...steps.map((s) => s.usage),
    transcript.turns.find((t) => t.turn === turn)?.totalUsage,
  ];
  const tokens = usages.reduce((sum, u) => sum + (u ? u.inputTokens + u.outputTokens : 0), 0);
  const toolCalls = transcript.toolCalls.filter((c) => c.turn === turn).length;
  const output = steps.some((s) => s.firstOutputMs !== undefined || s.text || s.reasoning);
  return tokens === 0 && toolCalls === 0 && !output ? 'infra-error' : 'agent-error';
}

export function errorText(error: unknown): string {
  if (error instanceof Error) return error.message;
  if (typeof error === 'string') return error;
  return JSON.stringify(error) ?? String(error);
}

/** Short human summary of a tool input for progress lines. */
export function summarizeInput(input: unknown): string {
  if (input && typeof input === 'object') {
    const obj = input as Record<string, unknown>;
    for (const key of ['command', 'cmd', 'file_path', 'filePath', 'path', 'url', 'query', 'pattern', 'skill']) {
      const value = obj[key];
      if (typeof value === 'string') return value;
      if (Array.isArray(value)) return value.join(' ');
    }
  }
  return typeof input === 'string' ? input : JSON.stringify(input) ?? '';
}
