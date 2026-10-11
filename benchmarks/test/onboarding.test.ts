import assert from 'node:assert/strict';
import test from 'node:test';

import { answerNumbers, liveDataCalls } from '../tasks/lib.js';
import type { ToolCallRecord, Transcript } from '../src/types.js';

let callId = 0;
function call(name: string, input: unknown, turn: number): ToolCallRecord {
  return { id: `c${callId++}`, turn, step: 0, name, input, output: '', isError: false, providerExecuted: false, startMs: 0 };
}

function transcript(toolCalls: ToolCallRecord[]): Transcript {
  return { turns: [], steps: [], toolCalls, errors: [] };
}

test('answer numbers accept thousands separators', () => {
  assert.deepEqual(answerNumbers('The current round is **435,570.**'), [435570]);
  assert.deepEqual(answerNumbers('round 435_570 or 435 570'), [435570, 435570]);
  assert.deepEqual(answerNumbers('round 435570, 12 miners, 3.79 SOL'), [435570]);
  assert.deepEqual(answerNumbers('1,2,3 and 999'), []);
});

test('live data calls count a4 get, a4 stream and Arete MCP in the requested turn only', () => {
  const calls = transcript([
    call('bash', { command: 'a4 get OreRound/latest --stack ore --limit 1' }, 1),
    call('bash', { command: 'a4 stream OreRound/latest --stack ore --first' }, 1),
    call('mcp__arete__read_view', { stack: 'ore', view: 'OreRound/latest' }, 1),
    call('bash', { command: 'a4 get OreRound/latest --stack ore --limit 1' }, 0),
    call('bash', { command: 'a4 explore stack ore --json' }, 1),
  ]);
  assert.deepEqual(liveDataCalls(calls, 1), ['a4 get', 'a4 stream', 'arete/read_view']);
  assert.deepEqual(liveDataCalls(calls, 0), ['a4 get']);
  assert.deepEqual(liveDataCalls(calls, 2), []);
});
