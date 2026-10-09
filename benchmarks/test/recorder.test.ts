import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import type { TextStreamPart, ToolSet } from 'ai';
import { Recorder, turnErrorStatus } from '../src/recorder.js';

async function* parts(list: unknown[]): AsyncIterable<TextStreamPart<ToolSet>> {
  for (const part of list) yield part as TextStreamPart<ToolSet>;
}

async function record(list: unknown[]): Promise<Recorder> {
  const recorder = new Recorder();
  recorder.beginTurn(0, 'prompt', false);
  await recorder.consume(parts(list));
  recorder.endTurn({ unfinished: false });
  return recorder;
}

const usage = { inputTokens: 1200, outputTokens: 40 };

describe('turnErrorStatus', () => {
  it('blames infra when the turn errors before any step', async () => {
    const recorder = await record([{ type: 'error', error: new Error('bad api key') }]);
    assert.equal(turnErrorStatus(recorder.transcript, 0), 'infra-error');
  });

  it('blames infra when an empty step started but the model did no work', async () => {
    const recorder = await record([
      { type: 'start-step', request: {}, warnings: [] },
      { type: 'error', error: new Error('provider unavailable') },
    ]);
    assert.equal(recorder.transcript.steps.length, 1);
    assert.equal(turnErrorStatus(recorder.transcript, 0), 'infra-error');
  });

  it('blames the agent once the model used tokens', async () => {
    const recorder = await record([
      { type: 'start-step', request: {}, warnings: [] },
      { type: 'finish-step', finishReason: 'stop', usage, response: {}, providerMetadata: undefined },
      { type: 'start-step', request: {}, warnings: [] },
      { type: 'error', error: new Error('context overflow') },
    ]);
    assert.equal(turnErrorStatus(recorder.transcript, 0), 'agent-error');
  });

  it('blames the agent once the model called a tool', async () => {
    const recorder = await record([
      { type: 'start-step', request: {}, warnings: [] },
      { type: 'tool-call', toolCallId: 't1', toolName: 'bash', input: { command: 'ls' } },
      { type: 'error', error: new Error('stream dropped') },
    ]);
    assert.equal(turnErrorStatus(recorder.transcript, 0), 'agent-error');
  });

  it('only looks at the errored turn', async () => {
    const recorder = new Recorder();
    recorder.beginTurn(0, 'first', false);
    await recorder.consume(parts([
      { type: 'start-step', request: {}, warnings: [] },
      { type: 'finish-step', finishReason: 'stop', usage, response: {}, providerMetadata: undefined },
    ]));
    recorder.endTurn({ unfinished: false });
    recorder.beginTurn(1, 'second', true);
    await recorder.consume(parts([
      { type: 'start-step', request: {}, warnings: [] },
      { type: 'error', error: new Error('provider unavailable') },
    ]));
    recorder.endTurn({ unfinished: false });
    assert.equal(turnErrorStatus(recorder.transcript, 1), 'infra-error');
  });
});
