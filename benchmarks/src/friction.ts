import type { ClassifiedCall } from './classify.js';
import { preview } from './recorder.js';
import type { Transcript } from './types.js';

export type FrictionKind =
  | 'failed-command'
  | 'cli-surface-mismatch'
  | 'help-lookup'
  | 'repeated-command'
  | 'doctor-not-ok'
  | 'self-update'
  | 'direct-api-call'
  | 'generated-sdk-edit'
  | 'credential-access'
  | 'slow-step'
  | 'tool-error'
  | 'turn-error';

export interface FrictionEvent {
  kind: FrictionKind;
  atMs: number;
  turn: number;
  step?: number;
  toolCallId?: string;
  summary: string;
  /** Excerpt of the command output or error. */
  evidence?: string;
  /** ms until the agent next succeeded at the same a4 command, if it did. */
  recoveredAfterMs?: number;
}

export interface FrictionReport {
  counts: Partial<Record<FrictionKind, number>>;
  events: FrictionEvent[];
}

const CLI_MISMATCH =
  /unrecognized subcommand|unexpected argument|unrecognized option|invalid value .* for|error: unknown|no such command|Did you mean|requires a subcommand|For more information, try '--help'/i;
const SLOW_STEP_MS = 90_000;

/**
 * Deterministic friction signals from a run: failed and repeated commands,
 * CLI/docs drift, help lookups, doctor warnings, direct API calls, edits to
 * generated SDK files, and slow model steps. These are the leads the session
 * reviewer and humans start from.
 */
export function detectFriction(transcript: Transcript, calls: ClassifiedCall[]): FrictionReport {
  const events: FrictionEvent[] = [];
  const add = (event: FrictionEvent) => events.push(event);
  const at = (c: ClassifiedCall) => ({
    atMs: c.record.startMs,
    turn: c.record.turn,
    step: c.record.step,
    toolCallId: c.record.id,
  });

  const seen = new Map<string, number>();
  for (const [index, call] of calls.entries()) {
    const evidence = preview(call.outputText.trim(), 600);

    if (call.command) {
      const key = call.command.replace(/\s+/g, ' ').trim();
      const count = (seen.get(key) ?? 0) + 1;
      seen.set(key, count);
      if (count === 3) add({ kind: 'repeated-command', ...at(call), summary: `ran 3+ times: ${preview(key, 200)}` });
    }

    if (call.failed) {
      const mismatch = call.a4.length > 0 && CLI_MISMATCH.test(call.outputText);
      const a4Path = call.a4[0]?.path;
      const recovery = a4Path
        ? calls.slice(index + 1).find((c) => !c.failed && c.a4.some((a) => a.path === a4Path))
        : undefined;
      add({
        kind: mismatch ? 'cli-surface-mismatch' : call.category === 'shell' ? 'failed-command' : 'tool-error',
        ...at(call),
        summary: `${call.record.name}: ${preview(call.command ?? JSON.stringify(call.record.input), 200)}`,
        evidence,
        ...(recovery ? { recoveredAfterMs: recovery.record.startMs - call.record.startMs } : {}),
      });
    }

    for (const a4 of call.a4) {
      if (a4.help) add({ kind: 'help-lookup', ...at(call), summary: `a4 ${a4.path} --help` });
      if (a4.path === 'self update') add({ kind: 'self-update', ...at(call), summary: a4.raw });
      if (a4.path === 'doctor' && !call.failed) {
        const status = /^\s{0,2}"status":\s*"(\w+)"/m.exec(call.outputText)?.[1];
        if (status && status !== 'ok') {
          add({ kind: 'doctor-not-ok', ...at(call), summary: `doctor status ${status}`, evidence });
        }
      }
    }

    if (call.directApi) {
      add({ kind: 'direct-api-call', ...at(call), summary: preview(call.command ?? '', 200) });
    }
    for (const path of call.editedPaths) {
      if (/(^|\/)(src\/arete|generated)\//.test(path)) {
        add({ kind: 'generated-sdk-edit', ...at(call), summary: path });
      }
    }
    const inputText = JSON.stringify(call.record.input ?? '');
    if (/credentials\.toml|a4_ak_|a4_sk_/.test(inputText)) {
      add({ kind: 'credential-access', ...at(call), summary: preview(call.command ?? inputText, 200) });
    }
  }

  for (const step of transcript.steps) {
    if (step.endMs === undefined) continue;
    const duration = step.endMs - step.startMs;
    if (duration > SLOW_STEP_MS) {
      add({
        kind: 'slow-step',
        atMs: step.startMs,
        turn: step.turn,
        step: step.step,
        summary: `model step took ${(duration / 1000).toFixed(0)}s`,
      });
    }
  }

  for (const error of transcript.errors) {
    add({ kind: 'turn-error', atMs: error.atMs, turn: error.turn, summary: preview(error.message, 300) });
  }

  events.sort((a, b) => a.atMs - b.atMs);
  const counts: FrictionReport['counts'] = {};
  for (const event of events) counts[event.kind] = (counts[event.kind] ?? 0) + 1;
  return { counts, events };
}
